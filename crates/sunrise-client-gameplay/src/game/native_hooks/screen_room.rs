use mod_api::ScreenView;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Collect left and right room from mods using the pre-reservation screen view.
    pub(in crate::game) fn native_screen_room(
        &self,
        screen: &ScreenView<'_>,
        left: &mut f32,
        right: &mut f32,
    ) {
        self.fire_screen_room(screen, left, right);
    }
}
