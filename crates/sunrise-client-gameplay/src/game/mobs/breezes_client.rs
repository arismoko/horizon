use crate::game::ClientGame;
use sunrise_items::blocks::RenderShape;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_sim::mobs::{Species, breeze};

impl ClientGame<'_> {
    pub fn tick_breeze_effects(&mut self) {
        let Some(camera) = self.common.player.as_ref().map(|p| p.position) else {
            return;
        };
        for i in 0..self.common.mobs.len() {
            let m = &self.common.mobs[i];
            if m.species() != Species::Breeze
                || !m.alive()
                || m.position.distance_squared(camera) > 32.0 * 32.0
            {
                continue;
            }
            let pose = breeze::pose(m);
            let (amount, jump_trail) = match pose {
                breeze::SLIDING => (20, false),
                breeze::LONG_JUMPING => (3, true),
                _ => (1, false),
            };
            if jump_trail {
                self.common.mobs[i].extra.scratch[1] += 1;
                if self.common.mobs[i].extra.scratch[1] > 5 {
                    continue;
                }
            } else {
                self.common.mobs[i].extra.scratch[1] = 0;
            }
            let (block, state) = self.common.breeze_ground(&self.common.mobs[i]);
            // `emitGroundParticles` skips an invisible block; the jump trail
            // (`emitJumpTrailParticles`) does not ask.
            if !jump_trail
                && self.common.res.facts().get(state).render_shape == RenderShape::Invisible
            {
                continue;
            }
            let Some((sprite, tint)) = self.particle_sprite(state, block) else {
                continue;
            };
            let m = &self.common.mobs[i];
            let at = if jump_trail {
                [
                    m.position.x + m.velocity.x,
                    m.position.y + m.velocity.y + f64::from(0.1_f32),
                    m.position.z + m.velocity.z,
                ]
            } else {
                [m.position.x, m.position.y, m.position.z]
            };
            // `ClientLevel.addParticle` with no speed: each crumb's own
            // generator draws its lifetime, its little push and the quarter of
            // the block's sprite it shows, never the level's random.
            let options = ParticleOptions::block(ParticleType::Block, sprite, tint);
            for _ in 0..amount {
                self.client.particles.0.add(&options, at, [0.0; 3]);
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    pub fn set_test_particle_camera(game: &mut crate::game::ClientGame<'_>, camera: [f64; 3]) {
        game.client.particles.0.camera = camera;
    }

    pub fn made_block_particles(game: &crate::game::ClientGame<'_>) -> u64 {
        game.client
            .particles
            .0
            .made()
            .get("block")
            .copied()
            .unwrap_or(0)
    }
}
