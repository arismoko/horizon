//! The `tooltip_render` area: host methods the game implements (`game/host/tooltip_render.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `tooltip_render` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait TooltipRenderClientHost {
    /// Side: client.
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key name (`key.keyboard.left.shift`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    fn is_key_down(&self, key: &str) -> bool {
        crate::phase::unprovided(
            "ClientHost::is_key_down",
            "is_key_down",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
