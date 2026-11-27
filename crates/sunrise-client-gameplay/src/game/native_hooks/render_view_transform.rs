use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    /// Presentation-only view transform at vanilla's `GameRenderer.bobHurt` / `bobView` stage.
    pub(in crate::game) fn native_render_view_transform(
        &self,
        partial: f32,
        transform: &mut mod_api::ViewTransform<'_>,
    ) {
        self.fire_render_view_transform(partial, transform);
    }
}
