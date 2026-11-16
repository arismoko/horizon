//! Initial-load publication boundary shared by startup and its resource fixture.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn publish_initial_client_models(&mut self) {
        if self.options.is_client_start() {
            // The ordinary launch loads the world tables on a worker while the title is shown,
            // but the renderers resolve against the mob types in them: wait for the worker, or
            // a launch that wins the race by a few milliseconds panics in `Resources::rest`.
            if let Err(error) = self.join_world_rest() {
                log::error!("Horizon: the world resources did not load: {error}");
                return;
            }
            self.resolve_entity_renderers();
            self.texture_atlases_stitched();
            self.native_model_baking_completed();
            self.native_client_setup();
        }
    }
}
