//! The `screen_exclusion_areas` area: host methods the game implements (`game/host/screen_exclusion_areas.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `screen_exclusion_areas` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait ScreenExclusionAreasClientHost {
    /// Side: client.
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `native_screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    fn screen_exclusion(&self) -> Vec<[f32; 4]> {
        crate::phase::unprovided(
            "ClientHost::screen_exclusion",
            "screen_exclusion",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
