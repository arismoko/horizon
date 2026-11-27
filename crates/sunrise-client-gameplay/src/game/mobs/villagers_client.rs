//! Client-side mob entity-event particle bursts (`Entity.handleEntityEvent`).
use crate::game::ClientGame;
use sunrise_level::particle_options::{
    Burst, ParticleOptions, ParticleType, around_self, entity_event_burst,
};

impl ClientGame<'_> {
    /// `handleEntityEvent(event)`'s particles about mob `id`.
    #[sunrise_mod_api_macros::replaceable]
    pub(in crate::game) fn entity_event_particles(&mut self, id: u32, event: i8) {
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        if let Some((kind, burst)) = entity_event_burst(&m.kind.id, event as u8) {
            self.particles_around_mob(id, kind, burst);
        }
    }

    #[sunrise_mod_api_macros::replaceable]
    fn particles_around_mob(&mut self, id: u32, kind: &'static str, burst: Burst) {
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let feet = [m.position.x, m.position.y, m.position.z];
        let size = [f64::from(m.width()), f64::from(m.height())];
        self.particles_around(kind, feet, size, burst);
    }

    #[sunrise_mod_api_macros::replaceable]
    fn particles_around(
        &mut self,
        kind: &'static str,
        feet: [f64; 3],
        size: [f64; 2],
        burst: Burst,
    ) {
        let Some(ty) = ParticleType::from_name(kind) else {
            return;
        };
        let options = ParticleOptions::simple(ty);
        let mut spawned = Vec::new();
        let particles = &mut self.client.particles.0;
        around_self(particles.random(), feet, size[0], size[1], burst, |p, v| {
            spawned.push((p, v));
        });
        for (p, v) in spawned {
            particles.add(&options, p, v);
        }
    }
}
