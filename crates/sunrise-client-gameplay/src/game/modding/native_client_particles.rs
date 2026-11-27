//! Local particle and firework requests from native client hooks.
use crate::game::ClientGame;
use mod_api::{
    Direction, FireworksRequest, LevelRef, LevelWriteError, client::particles::PositionSource,
};
use sunrise_client::particles::{Particle, ParticleLevel};
use sunrise_level::particle_options::ParticleOptions;
use sunrise_world::registry::BlockStateId;

fn finite_options(options: &ParticleOptions) -> bool {
    options.color.into_iter().all(f32::is_finite)
        && options.to_color.into_iter().all(f32::is_finite)
        && options.scale.is_finite()
        && options
            .trail
            .as_ref()
            .is_none_or(|(at, _)| at.iter().all(|v| v.is_finite()))
        && options
            .geyser
            .is_none_or(|(_, impulse)| impulse.is_finite())
        && options.vibration.as_ref().is_none_or(|(source, at, _)| {
            at.iter().all(|v| v.is_finite())
                && match source {
                    PositionSource::Block(_) => true,
                    PositionSource::Entity { y_offset, .. } => y_offset.is_finite(),
                    _ => false,
                }
        })
}

impl ClientGame<'_> {
    /// `ClientLevel.addParticle`: request one particle in this client's loaded world.
    /// Options and provider input slots pass through unchanged, including encoded
    /// color or size values in `velocity`. Normal distance, settings and population
    /// limits still apply. True means submitted, not necessarily visible: an unknown
    /// provider or missing sprite set draws nothing. False on a server, without a
    /// client player/level, or for non-finite numeric input. API @API:client_particles@.
    pub(in crate::game) fn add_particle(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> bool {
        if !self.common.native_window()
            || self.common.player.is_none()
            || !at.into_iter().all(f64::is_finite)
            || !velocity.into_iter().all(f64::is_finite)
            || !finite_options(options)
        {
            return false;
        }
        // The limiter's roll draws the level random: inside a part's `animate_tick`, the one
        // the part holds as `event.random`.
        self.with_lent_level_random(|game| {
            let level = ParticleLevel::new(
                &game.common.world,
                game.common.res.facts(),
                game.common.collision,
            );
            game.client
                .particles
                .0
                .add_with(Some(&level), options, at, velocity, false, false);
        });
        true
    }
    pub(in crate::game) fn level_add_breaking_particles(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        face: Direction,
    ) -> Result<bool, LevelWriteError> {
        if self.native_client_level() != Some(level) {
            return Err(LevelWriteError::Unavailable);
        }
        Ok(self.add_breaking_particle(pos, state, face))
    }

    pub(in crate::game) fn level_add_terrain_particle(
        &mut self,
        level: LevelRef,
        state: BlockStateId,
        source_pos: [i32; 3],
        at: [f64; 3],
        velocity: [f64; 3],
    ) -> Result<bool, LevelWriteError> {
        if self.native_client_level() != Some(level) {
            return Err(LevelWriteError::Unavailable);
        }
        if !at.into_iter().chain(velocity).all(f64::is_finite) {
            return Ok(false);
        }
        let Some((sprite, tint)) = self.particle_sprite(state, source_pos) else {
            return Ok(false);
        };
        let particle =
            Particle::terrain(self.client.particles.0.random(), at, velocity, sprite, tint);
        self.client.particles.0.push(particle);
        Ok(true)
    }

    /// `ClientLevel.createFireworks`: queue a firework's explosions in this client's loaded
    /// world through the starter a bursting rocket uses, its blast and twinkle sounds gated by
    /// `audible` as a rocket's by its silence. No entity, no damage, nothing on the server.
    /// True means queued; false on a server, without a client player/level or for non-finite
    /// input. API @API:client_fireworks@.
    pub(in crate::game) fn create_fireworks(&mut self, request: &FireworksRequest) -> bool {
        use sunrise_gameplay_common::game::api_values::TryIntoEngine;
        if !self.common.native_window()
            || self.common.player.is_none()
            || !request
                .at
                .into_iter()
                .chain(request.velocity)
                .all(f64::is_finite)
        {
            return false;
        }
        let Ok(explosions) = request
            .explosions
            .iter()
            .map(|explosion| explosion.try_into_engine(&()))
            .collect();
        let camera = self.common.eye_position().unwrap_or(request.at);
        // Without explosions its poofs are added at once, each with the limiter's roll.
        self.with_lent_level_random(|game| {
            game.client.particles.0.fireworks(
                request.at,
                request.velocity,
                explosions,
                camera,
                request.audible,
            );
        });
        true
    }
}

pub(in crate::game) fn particle_block_states() -> Option<sunrise_client::particles::BlockStates> {
    sunrise_gameplay_common::game::api_values::block_state::current()
        .ok()
        .map(|states| -> sunrise_client::particles::BlockStates { states })
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_level::particle_options::ParticleType;

    #[test]
    fn nonfinite_nested_particle_parameters_are_rejected() {
        let good = ParticleOptions::simple(ParticleType::Smoke);
        assert!(finite_options(&good));
        let mut bad = good.clone();
        bad.to_color[0] = f32::NAN;
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.trail = Some(([f64::INFINITY, 0.0, 0.0], 3));
        assert!(!finite_options(&bad));
        bad = good.clone();
        bad.geyser = Some((1, f32::NAN));
        assert!(!finite_options(&bad));
        bad = good;
        bad.vibration = Some((
            PositionSource::Entity {
                id: 1,
                y_offset: f32::NAN,
            },
            [0.0; 3],
            3,
        ));
        assert!(!finite_options(&bad));
        bad.vibration = Some((PositionSource::Block([0; 3]), [0.0, f64::NAN, 0.0], 3));
        assert!(!finite_options(&bad));
    }
}
