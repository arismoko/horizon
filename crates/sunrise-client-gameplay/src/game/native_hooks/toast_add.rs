use sunrise_game_data::text::Text;

/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `ToastAddEvent`: admit one client toast before it enters a visible queue.
    pub(in crate::game) fn native_toast_add(
        &self,
        kind: mod_api::ToastKind,
        title: &mut Text,
        description: &mut Option<Text>,
        cancel: &mut bool,
    ) {
        self.fire_toast_add(kind, title, description, cancel);
    }
}
