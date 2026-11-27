use mod_api::CollisionExclusionsError;
use sunrise_gameplay_common::game::native_collision_exclusions::refused;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// For a client mod: the local player's collision exclusions, in place of the last ones.
    /// Refused with no player of its own, and for boxes the set refuses; the set there was stays.
    pub fn set_collision_exclusions(
        &mut self,
        boxes: &[[f64; 6]],
    ) -> Result<(), CollisionExclusionsError> {
        let body = self
            .common
            .player
            .as_mut()
            .ok_or(CollisionExclusionsError::Stale)?;
        body.collision_exclusions.set(boxes).map_err(refused)
    }
}
