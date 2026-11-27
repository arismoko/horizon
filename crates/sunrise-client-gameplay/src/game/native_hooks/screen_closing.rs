use mod_api::ScreenView;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// The `screen` that was open is being replaced (`ScreenEvent.Closing`), after `native_screen_opening` let it.
    pub(in crate::game) fn native_screen_closing(&self, screen: &ScreenView<'_>) {
        self.fire_screen_closing(screen);
    }
}
