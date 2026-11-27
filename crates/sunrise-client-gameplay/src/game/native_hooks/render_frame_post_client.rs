use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    pub fn finish_render_frame(&mut self, elapsed: Duration) {
        let (world_partial_tick, partial_tick) = self
            .client
            .extras
            .frame_partial_ticks
            .take()
            .unwrap_or_else(|| self.frame_partial_ticks());
        self.native_render_frame_post(elapsed, world_partial_tick, partial_tick);
    }
}
