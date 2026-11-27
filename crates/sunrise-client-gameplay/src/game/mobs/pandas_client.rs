use crate::game::ClientGame;
use sunrise_items::items::ItemStack;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_sim::mobs::{Species, panda};

impl ClientGame<'_> {
    /// What a window draws of its pandas: the crumbs of a bite
    /// (`addEatingParticles`) and the sneeze's particle.
    pub(in crate::game::client_side) fn panda_effects(&mut self) {
        let mut bites = Vec::new();
        let mut sneezes = Vec::new();
        for m in self
            .common
            .mobs
            .iter_mut()
            .filter(|m| m.species() == Species::Panda)
        {
            let (crumbs, sneeze) = panda::effects(m);
            if let (Some(crumbs), Some(item)) = (
                crumbs,
                sunrise_sim::mobs::finalize::item(m, sunrise_sim::mobs::finalize::MAINHAND),
            ) {
                bites.push((item, crumbs));
            }
            sneezes.extend(sneeze);
        }
        for (item, crumbs) in bites {
            let options = self.item_particle(&ItemStack::new(&item, 1));
            for (at, velocity) in crumbs {
                self.client.particles.0.add(&options, at, velocity);
            }
        }
        let options = ParticleOptions::simple(ParticleType::Sneeze);
        for (at, velocity) in sneezes {
            self.client.particles.0.add(&options, at, velocity);
        }
    }
}
