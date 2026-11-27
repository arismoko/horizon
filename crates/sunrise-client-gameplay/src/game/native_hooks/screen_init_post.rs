use mod_api::{ScreenView, ScreenWidgets};

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// A screen initialized or resized (`ScreenEvent.Init.Post`).
    pub(in crate::game) fn native_screen_init_post(
        &self,
        screen: &ScreenView<'_>,
        stage_listener: impl FnMut(&mut dyn FnMut(&mut dyn ScreenWidgets) -> bool),
    ) {
        self.fire_screen_init_post(screen, stage_listener);
    }
}
