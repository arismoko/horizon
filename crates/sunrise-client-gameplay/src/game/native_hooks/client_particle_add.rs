use sunrise_level::particle_options::ParticleOptions;

/// `ClientLevel.doAddParticle`'s head: the client's mods may move, re-aim or drop the
/// particle `options` asks for at `pos` with `velocity` before the camera distance check
/// and the Particles setting decide whether it is created. The particle engine calls it
/// through its `particle_add` slot, which `ClientGame::sync_render_hooks` fills only while
/// an enabled mod listens.
pub fn native_client_particle_add(
    options: &ParticleOptions,
    pos: &mut [f64; 3],
    velocity: &mut [f64; 3],
    override_limiter: bool,
    always_show: bool,
    cancel: &mut bool,
) {
    crate::game::native_hooks::fire::fire_client_particle_add(
        options,
        pos,
        velocity,
        override_limiter,
        always_show,
        cancel,
    );
}
