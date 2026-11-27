/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Once a frame, in a world or not, right after the frame was rendered
    /// (`RenderFrameEvent.Post`).
    pub(in crate::game) fn native_render_frame_post(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
    ) {
        self.fire_render_frame_post(elapsed, world_partial_tick, partial_tick);
    }
}
