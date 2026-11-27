use super::*;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ClientGame<'_> {
    pub(crate) fn frame_world_partial_tick(&self) -> f64 {
        self.client.extras.frame_partial_ticks.map_or_else(
            || self.common.clock.fraction(),
            |(world, _)| f64::from(world),
        )
    }
}
