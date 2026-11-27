use super::*;

impl crate::game::ServerGame<'_> {
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), including eviction of idle columns while the level
    /// remains open. Don't read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `level_unload` and server shutdown
    /// (`server_stopping` / `server_stopped`) tell.
    /// Server-side.
    pub fn native_chunk_unload(&mut self, dimension: &str, chunk: [i32; 2]) {
        self.fire_chunk_unload(dimension, chunk);
        if let Some(level) = self.level(dimension) {
            self.forget_composter_chunk(level.dimension(), chunk);
            self.unload_water_tickets_chunk(level, chunk);
        }
    }
    /// Chunk `chunk` of this server level `dimension` left it, after its last save: the
    /// parts of its mod block entities are told, then the mods of the chunk.
    pub(in crate::game) fn native_chunk_and_block_entities_unload(
        &mut self,
        dimension: &str,
        chunk: [i32; 2],
    ) {
        if mod_api::__private::has_part_hook(mod_api::HookId::OnChunkUnloaded)
            && let Some(level) = self.level(dimension)
        {
            self.notify_chunk_block_entities_unloaded(level, (chunk[0], chunk[1]));
        }
        self.native_chunk_unload(dimension, chunk);
    }
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::on_chunk_unloaded::OnChunkUnloadedGameExt2 as _;
