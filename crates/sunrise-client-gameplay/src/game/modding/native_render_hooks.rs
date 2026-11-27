//! The render crate's mod hook slots (`sunrise_client_render::mod_hooks`), filled while a mod listens.
use crate::game::ClientGame;
use mod_api::HookId;
use sunrise_client_render::mod_hooks::{ModHooks, ParticleAdd};
use sunrise_level::particle_options::ParticleOptions;

impl ClientGame<'_> {
    /// Fill each render-crate hook slot while an enabled mod of this window listens to its
    /// hook, and clear it otherwise. Run once per client tick, so a mod switched off or
    /// unloaded stops costing its hook from the next tick on; the dispatch itself already
    /// skips a switched-off mod in between.
    pub(in crate::game) fn sync_render_hooks(&mut self) {
        let window = self.common.native_window();
        let listened = |hook| window && mod_api::__private::enabled_listened(hook);
        self.client.particles.0.set_mod_hooks(ModHooks {
            particle_add: listened(HookId::ClientParticleAdd)
                .then_some(particle_add as ParticleAdd),
        });
    }
}

/// [`ModHooks::particle_add`]: `client_particle_add`. The dispatch contains a panicking
/// listener and keeps the particle as that listener found it.
fn particle_add(
    options: &ParticleOptions,
    pos: &mut [f64; 3],
    velocity: &mut [f64; 3],
    override_limiter: bool,
    always_show: bool,
) -> bool {
    let mut cancel = false;
    crate::game::native_hooks::client_particle_add::native_client_particle_add(
        options,
        pos,
        velocity,
        override_limiter,
        always_show,
        &mut cancel,
    );
    !cancel
}
