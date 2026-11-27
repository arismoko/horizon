#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Client camera angles before `Camera.setRotation`, third-person mirroring, detached
    /// back-off, and sleeping bed rotation (`ViewportEvent.ComputeCameraAngles`).
    pub(in crate::game) fn native_compute_camera_angles(
        &self,
        partial: f32,
        yaw: &mut f32,
        pitch: &mut f32,
        roll: &mut f32,
    ) {
        self.fire_compute_camera_angles(partial, yaw, pitch, roll);
    }
}
