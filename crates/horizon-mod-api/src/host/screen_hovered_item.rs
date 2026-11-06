//! The `screen_hovered_item` area: host methods the game implements (`game/host/screen_hovered_item.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `screen_hovered_item` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait ScreenHoveredItemClientHost {
    /// Side: client.
    /// The id of the item in the slot under the pointer on the open container screen
    /// (`AbstractContainerScreen.getHoveredSlot().getItem()`), None over no slot or an empty one.
    fn screen_hovered_item(&self) -> Option<String> {
        crate::phase::unprovided(
            "ClientHost::screen_hovered_item",
            "screen_hovered_item",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
