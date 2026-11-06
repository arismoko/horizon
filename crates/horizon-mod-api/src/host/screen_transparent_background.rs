//! The `screen_transparent_background` area: host methods the game implements (`game/host/screen_transparent_background.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `screen_transparent_background` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait ScreenTransparentBackgroundClientHost {
    /// Side: client.
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    fn apply_screen_blur(&self) -> bool {
        crate::phase::unprovided(
            "ClientHost::apply_screen_blur",
            "apply_screen_blur",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
    /// Side: client.
    /// The user's configured menu background blur radius (`menuBackgroundBlurriness` in options.txt,
    /// usually 0.0 to 10.0; default 5.0). 0.0 means blur is turned off. API @API:screen_transparent_background@.
    fn menu_blur_radius(&self) -> f32 {
        crate::phase::unprovided(
            "ClientHost::menu_blur_radius",
            "menu_blur_radius",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
