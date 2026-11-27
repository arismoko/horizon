//! The `register_particle_providers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Registers logical-client sprite-set factories for already registered [`ParticleType`]s
    /// (NeoForge's `RegisterParticleProvidersEvent`). Each provider creates owned particles
    /// from typed options. The last successful registration per type wins. Sprite reload
    /// drops live and pending instances and binds the factories to the new sprite sets.
    /// If a listener panics or a phase call is refused, none of its registrations take effect.
    #[notes = r#"## Particle providers

Register a common `ParticleType<T>` in `Hooks::register`, supplying its map and stream codecs.
`Hooks::register_particle_providers` registers the client factory with
`event.providers.register_sprite_set(ty, |sprites| Provider::new(sprites))`. The matching sprite
list is `assets/<namespace>/particles/<path>.json`. Spawn using `ty.options(value)`; a unit type
uses `ParticleMapCodec::unit()`, `ParticleStreamCodec::wire()` and `ty.options(())`.

The provider's `create_particle` receives a `ParticleSpawn<T>` containing a borrowed typed value,
client level, position, auxiliary inputs and provider RNG, plus a writable `ParticleTickView`.
It can edit the initialized particle and returns an owned `CustomParticle`, or `None` to decline.
The engine still owns the camera-distance, Particles-setting and group admission rules.
The instance's fields are its private state. It gets a live view on each tick; the default
runs `Particle.tick`. An override can advance age separately, move the particle and remove it.
Render output never decides whether a particle is alive. No instance or view crosses the network.

`CustomParticle::extract` borrows the particle and the current frame. The default submits the
normal quad with its sprite, lighting and interpolated pose. An override can emit zero or
more independently placed opaque or translucent quads. An invalid quad is omitted; a failed
callback does not commit that call's geometry. Resource reload does not replay the hook."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_particle_providers::native_register_particle_providers",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 4,
        contract = "listen",
        staged_surface = "providers",
        context = "registration",
        neoforge = "RegisterParticleProvidersEvent"
    )]
    fn register_particle_providers(providers: &mut dyn ParticleProviders);
}
