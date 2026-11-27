use crate::game::ClientGame;

impl ClientGame<'_> {
    /// Render-only world-space camera position after `Camera.alignWithEntity` places the camera.
    pub(in crate::game) fn native_camera_position(
        &self,
        partial: f32,
        position: &mut mod_api::CameraPosition,
    ) {
        self.fire_camera_position(partial, position);
    }
}
