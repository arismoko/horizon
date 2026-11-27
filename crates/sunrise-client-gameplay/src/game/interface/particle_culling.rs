//! The window's particle culling: this frame's mode, and the worker's frame over the game's fields.
use crate::game::ClientGame;
use sunrise_client::graphics::ParticleCullingMode;
use sunrise_client_gameplay::particle_culling::state_is_opaque;
pub(in crate::game) use sunrise_client_gameplay::particle_culling::{
    ParticleAsyncState, ParticleOcclusionContext,
};

/// A `SectionScene` of the game's fields, so `async_particle_culler` can be borrowed beside it.
macro_rules! section_scene {
    ($game:expr) => {
        sunrise_client_gameplay::section_scene::SectionScene {
            world: &$game.world,
            facts: $game.res.facts(),
            renderer: $game.client.renderer.0.as_deref(),
            streaming: &$game.client.streaming,
            pending_uploads: &$game.client.pending_uploads.0,
        }
    };
}
pub(in crate::game) use section_scene;

impl ClientGame<'_> {
    pub(in crate::game::client_side) fn particle_occlusion_context(
        &self,
        camera: glam::DVec3,
    ) -> ParticleOcclusionContext<'_> {
        let at = camera.to_array().map(|value| value.floor() as i32);
        let camera_inside_opaque = state_is_opaque(
            self.common
                .res
                .facts()
                .get(self.common.world.block(at[0], at[1], at[2])),
        );
        let mode = if self.model_packs_are_vanilla()
            && self.common.player.is_some()
            && !camera_inside_opaque
        {
            self.client.settings.sunrise.particle_culling
        } else {
            ParticleCullingMode::Off
        };
        ParticleOcclusionContext::new(
            section_scene!(self),
            &self.client.async_particle_culler.0,
            mode,
            camera,
        )
    }

    pub(in crate::game::client_side) fn finish_async_particle_culling(&self) {
        self.client
            .async_particle_culler
            .0
            .borrow_mut()
            .finish_frame(&self.common.world, self.common.res.facts());
    }
}
