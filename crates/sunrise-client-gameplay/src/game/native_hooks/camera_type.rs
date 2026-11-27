/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `key.togglePerspective` (F5) cycles the camera type: `previous` is the type it
    /// leaves, and `next` comes in as `CameraType.cycle` and may be set to another.
    pub(in crate::game) fn native_camera_type_cycle(
        &mut self,
        previous: mod_api::CameraType,
        next: &mut mod_api::CameraType,
    ) {
        self.fire_camera_type_cycle(previous, next);
    }
}
