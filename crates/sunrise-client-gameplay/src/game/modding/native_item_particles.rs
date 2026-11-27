//! Reuse the game's stack model resolution for mod particle requests.
use crate::game::ClientGame;
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::ParticleOptions;

impl ClientGame<'_> {
    pub fn item_particle_options(&self, stack: &ItemStack) -> Option<ParticleOptions> {
        Some(self.item_particle(stack))
    }
}
