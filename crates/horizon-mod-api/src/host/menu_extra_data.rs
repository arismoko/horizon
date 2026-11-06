//! The `menu_extra_data` area: host methods the game implements (`game/host/menu_extra_data.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `menu_extra_data` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait MenuExtraDataClientHost {
    /// Side: client.
    /// The block of the mod screen that is open (`native_screen`), as the
    /// server named it when the block opened its menu type; None when no mod
    /// screen is open. 401.
    fn screen_pos(&self) -> Option<[i32; 3]> {
        crate::phase::unprovided(
            "ClientHost::screen_pos",
            "screen_pos",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// What the block's mod wrote for the mod screen that is open
    /// (`native_menu_extra_data` on the server), empty when it wrote nothing;
    /// None when no mod screen is open. @API:menu_extra_data@.
    /// Not on wasm: it returns a borrowed view of the screen's bytes, which crosses only as a resource.
    fn screen_extra_data(&self) -> Option<&[u8]> {
        crate::phase::unprovided(
            "ClientHost::screen_extra_data",
            "screen_extra_data",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
