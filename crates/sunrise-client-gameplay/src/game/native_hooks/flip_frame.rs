/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Once for each frame presented, after `render_frame_post` (`FlipFrameEvent`).
    pub(in crate::game) fn native_flip_frame(&mut self) {
        self.fire_flip_frame();
    }
}
