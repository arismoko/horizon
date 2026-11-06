//! The `block_render_visibility` area: host methods the game implements (`game/host/block_render_visibility.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `block_render_visibility` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait BlockRenderVisibilityClientHost {
    /// Side: client.
    /// Asks `native_block_render_visibility` again for every state and, when the answers differ
    /// from those in use, remeshes every loaded section with them. Call it when your answer changes (a toggle key, a scan
    /// finishing) and once when the player joins, not every frame: a rebuild takes the workers a
    /// few seconds. With no part answering, nothing is asked and the mesher keeps its no-mod path.
    fn refresh_render_visibility(&mut self) {
        crate::phase::unprovided(
            "ClientHost::refresh_render_visibility",
            "refresh_render_visibility",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
