use mod_api::{LevelRef, RandomDraw};
use sunrise_core::random::LegacyRandom;
use sunrise_gameplay_common::game::api_values::with_live_level_random;

impl crate::game::ServerGame<'_> {
    /// Lend the named level's live stream directly.
    pub(in crate::game) fn with_level_random(&mut self, level: LevelRef, draw: RandomDraw<'_>) {
        let Some(random) = self.level_random_mut(level) else {
            return;
        };
        with_live_level_random(random, draw);
    }

    /// Select a current, resident server level's stream without bringing the level into place.
    pub(in crate::game) fn level_random_mut(
        &mut self,
        level: LevelRef,
    ) -> Option<&mut LegacyRandom> {
        if !self.common.is_logical_server() || !self.level_handle_current(level) {
            return None;
        }
        let world = if level.dimension() == self.common.dim.key() {
            &mut self.common.world
        } else {
            self.server
                .levels
                .0
                .parked_world_mut_of(level.dimension_id())?
        };
        Some(&mut world.random)
    }
}
