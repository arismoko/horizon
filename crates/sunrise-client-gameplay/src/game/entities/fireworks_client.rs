use super::item_draw::owner_at;
use sunrise_assets::item_model::DisplayContext;
use sunrise_client::{block_entity_render::vertex_light, mesher::Vertex};
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::fireworks::Rocket;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};

impl crate::game::ClientGame<'_> {
    /// `FireworkEntityRenderer.submit`: ground item facing camera, lit at the rocket's probe.
    pub fn rocket_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        // shouldRender: not while attached to an entity (`isAttachedToEntity`).
        for r in self.common.rockets[run]
            .iter()
            .filter(|r| !r.attached && r.attached_to.is_none() && !self.hides_as_camera(r.id))
        {
            // FireworkRocketEntity.shouldRenderAtSqrDistance: 0.25 box within 64 blocks.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(r.previous),
                super::entity_culling::at(r.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = glam::DVec3::new(
                r.previous.x + (r.position.x - r.previous.x) * alpha,
                r.previous.y + (r.position.y - r.previous.y) * alpha,
                r.previous.z + (r.position.z - r.previous.z) * alpha,
            );
            // `updateForNonLiving`: GROUND, owned by the rocket.
            let owner = owner_at(p.to_array(), 0.0, "minecraft:firework_rocket");
            let state = self.resolve_item(&r.item, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let mut pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            if r.angled {
                pose *= glam::Mat4::from_rotation_z(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
            }
            // getLightProbePosition: the eye, 0.85 of its 0.25 height.
            let light = self.common.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            // Frame generation and temporal upscaling follow its own flight.
            self.mark_motion(
                out.len(),
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(r.id),
            );
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
        self.mark_motion(out.len(), 0);
    }
    /// FireworkRocketEntity.handleEntityEvent: use the synchronized item data.
    pub(in crate::game::client_side) fn remote_firework_explosion(&mut self, id: i32) {
        let camera = self.common.eye_position();
        if let Some(r) = self.common.rockets.iter().find(|r| r.id == id as u32) {
            let at = [r.position.x, r.position.y, r.position.z];
            let v = [r.velocity.x, r.velocity.y, r.velocity.z];
            self.client.particles.0.fireworks(
                at,
                v,
                r.explosions.as_ref().clone(),
                camera.unwrap_or(at),
                !r.silent,
            );
        }
    }
    /// The trail (`FireworkRocketEntity.tick` on a client): a spark each tick
    /// at the rocket, drifting against its motion.
    pub(in crate::game::client_side) fn rocket_spark(&mut self, r: &Rocket) {
        let g = |g: &mut sunrise_core::random::LegacyRandom| g.next_gaussian() * 0.05;
        let v = [
            g(&mut self.common.world.random),
            -r.velocity.y * 0.5,
            g(&mut self.common.world.random),
        ];
        let at = [r.position.x, r.position.y, r.position.z];
        self.client
            .particles
            .0
            .add(&ParticleOptions::simple(ParticleType::Firework), at, v);
    }
}
