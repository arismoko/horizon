use super::block_entity_read::ResidentReadBlockEntity;
use super::*;
use mod_api::BlockEntityDataView;
use sunrise_level::containers::BlockEntity;
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    fn native_on_chunk_unloaded(
        &mut self,
        pos: [i32; 3],
        block_entity: BlockEntityRef,
        entity: BlockEntityDataView<'_>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_on_chunk_unloaded(
            self,
            pos,
            block_entity,
            entity,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

pub trait OnChunkUnloadedGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// Tells the parts of the mod block entities in chunk `chunk` of `level` that it
    /// unloads, each once, while they are still in the world's block entities.
    fn notify_chunk_block_entities_unloaded(
        &mut self,
        level: mod_api::LevelRef,
        chunk: (i32, i32),
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded) {
            return;
        }
        // Keep the outgoing set and original generations across callbacks. The loans give the
        // mutable side Host no alias to these rows, and hand them back for normal column cleanup.
        let block_entities = &mut self.world.block_entities;
        let positions: Vec<_> = block_entities.mods_in(chunk).copied().collect();
        let unloading: Vec<_> = positions
            .into_iter()
            .filter_map(|key @ (x, y, z)| {
                let generation = block_entities.generation(&key)?;
                let (lease, entity) = block_entities.take(&key)?;
                let pos = [x, y, z];
                let handle = mod_api::__private::game::block_entity_ref(level, pos, generation);
                Some((pos, handle, lease, entity))
            })
            .collect();
        for (pos, handle, lease, row) in unloading {
            if let BlockEntity::Mod(entity) = &row {
                let data = ResidentReadBlockEntity::stored(entity);
                self.native_on_chunk_unloaded(pos, handle, BlockEntityDataView::new(&data));
            }
            self.world.block_entities.put(lease, row);
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> OnChunkUnloadedGameExt2<'__sg> for G {}
