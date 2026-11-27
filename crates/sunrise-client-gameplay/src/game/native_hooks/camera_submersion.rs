#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `Camera.getFluidInCamera`: the fog type at `camera`, after `state_at_viewpoint`; the mods
    /// may replace it.
    pub(in crate::game) fn native_camera_submersion(
        &self,
        camera: [f64; 3],
        fog_type: &mut mod_api::client::fog::FogType,
    ) {
        self.fire_camera_submersion(camera, fog_type);
    }
}
