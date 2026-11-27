//! `ClientRenderClientHost::set_block_extra_models`: the standalone models a client mod draws as
//! part of a block (NeoForge's `ModelData` use), kept in the client's columns for the chunk mesher
//! (`World::set_extra_models`), which reads them on its workers and calls no mod code.
