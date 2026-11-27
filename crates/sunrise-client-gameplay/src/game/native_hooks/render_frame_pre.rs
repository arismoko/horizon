/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn frame_partial_ticks(&self) -> (f32, f32) {
        let partial_tick = self.common.clock.window_fraction();
        let world_partial_tick = if !self.shows_panorama() && self.common.player.is_some() {
            self.common.clock.fraction() as f32
        } else {
            partial_tick
        };
        (world_partial_tick, partial_tick)
    }

    /// Once a frame, in a world or not, after Sunrise's extras and just
    /// before the frame is rendered (`RenderFrameEvent.Pre`).
    pub fn native_render_frame_pre(
        &mut self,
        elapsed: Duration,
        world_partial_tick: f32,
        partial_tick: f32,
    ) {
        self.fire_render_frame_pre(elapsed, world_partial_tick, partial_tick);
    }
}
