use crate::game::ClientGame;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{block_entity_render::vertex_light, item_render, mesher::Vertex};
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::ominous_item_spawner_common::{ROTATION_SPEED, TICKS_SCALING};
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl ClientGame<'_> {
    /// `OminousItemSpawnerRenderer`: full-bright item growing over its first 50 ticks and turning.
    pub(in crate::game) fn ominous_item_spawner_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        for s in &self.common.ominous_spawners {
            if self.hides_as_camera(s.id) {
                continue;
            }
            let at = glam::DVec3::from_array(s.position);
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let state = self.resolve_item(&s.item, DisplayContext::Ground, None, 0);
            if state.is_empty() {
                continue;
            }
            let age = s.tick_count as f32 + alpha as f32;
            let scale = if age <= TICKS_SCALING {
                age.min(TICKS_SCALING) / TICKS_SCALING
            } else {
                1.0
            };
            let spin = (age * ROTATION_SPEED).rem_euclid(360.0);
            let pose = glam::Mat4::from_translation((at - camera).as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::splat(scale))
                * glam::Mat4::from_rotation_y(spin.to_radians());
            let seed = i64::from(self.common.item_number(&s.item.item).unwrap_or(0))
                + i64::from(s.item.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose,
                item_render::rendered_amount(s.item.count),
                seed,
            ) {
                self.submit_item_world(&state, copy, vertex_light(0xFF), out);
            }
        }
    }
    /// `OminousItemSpawner.addParticles`: one to three ominous spawning
    /// particles from the spawner out to points around it.
    pub(in crate::game) fn ominous_spawner_particles(&mut self, at: [f64; 3]) {
        assert!(
            !self.is_logical_server(),
            "ominous_spawner_particles called on Server role"
        );
        let options = ParticleOptions::simple(ParticleType::OminousSpawning);
        let r = self.client.particles.0.random();
        let count = r.next_int_between_inclusive(1, 3).unwrap_or(1);
        let mut flights = Vec::new();
        for _ in 0..count {
            let mut spread = || 0.4 * (r.next_gaussian() - r.next_gaussian());
            let from = [at[0] + spread(), at[1] + spread(), at[2] + spread()];
            flights.push([from[0] - at[0], from[1] - at[1], from[2] - at[2]]);
        }
        for v in flights {
            self.client.particles.0.add(&options, at, v);
        }
    }
    /// `OminousItemSpawner.tick` on a client: advances tickCount and emits particles every 5 ticks.
    pub fn tick_remote_ominous_item_spawners(&mut self) {
        let at: Vec<[f64; 3]> = self
            .common
            .ominous_spawners
            .iter()
            .map(|s| s.position)
            .collect();
        self.common
            .ominous_spawners
            .for_each_mut(|s| s.tick_count += 1);
        if self.common.world.game_time % 5 == 0 {
            for position in at {
                self.ominous_spawner_particles(position);
            }
        }
    }
}
