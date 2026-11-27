use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    pub(in crate::game) fn apply_screen_blur(&self) -> bool {
        if self.client.screen_blur_requested.get() {
            return false;
        }
        self.client.screen_blur_requested.set(true);
        true
    }
}
