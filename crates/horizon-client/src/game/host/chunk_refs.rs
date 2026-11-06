//! The game's side of the `chunk_refs` area (`horizon-mod-api/src/host/chunk_refs.rs`).
use crate::game::Game;
use mod_api::{prelude::*, *};
impl ChunkRefsHost for Game<'_> {
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        mod_api::__private::check_access("chunk", mod_api::__private::LoadAccess::WorldOnly);
        self.native_chunk(level, pos)
    }
}
impl ChunkRefsBackend for Game<'_> {
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool {
        mod_api::__private::check_access("chunk_loaded", mod_api::__private::LoadAccess::WorldOnly);
        self.native_chunk_ref_loaded(chunk)
    }
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus> {
        mod_api::__private::check_access("chunk_status", mod_api::__private::LoadAccess::WorldOnly);
        self.native_chunk_status(chunk)
    }
    fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>> {
        mod_api::__private::check_access(
            "level_loaded_chunks",
            mod_api::__private::LoadAccess::WorldOnly,
        );
        self.native_level_loaded_chunks(level)
    }
}
