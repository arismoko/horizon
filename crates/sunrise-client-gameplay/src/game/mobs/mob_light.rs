//! The light a mob is drawn with.
use crate::game::ClientGame;
use sunrise_sim::mobs::Mob;

impl ClientGame<'_> {
    /// `EntityRenderer.getPackedLightCoords`: the light `mob` is drawn with at `alpha`.
    pub(in crate::game::client_side) fn mob_packed_light(&self, mob: &Mob, alpha: f64) -> u32 {
        sunrise_client_gameplay::mob_light::packed_light(&self.common.world, mob, alpha)
    }
}
