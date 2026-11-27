//! The `register_particle_groups` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the render groups a mod's particles draw in (NeoForge's `RegisterParticleGroupsEvent`).
    /// Called once at client startup. Use [`RegisterParticleGroupsEvent::register`] to bind an ID to
    /// a factory run once per particle engine construction. Sprite loads and reloads only resolve
    /// each particle type against the constructed groups.
    #[notes = r#"## Particle groups

`Hooks::register_particle_groups` is the client registration-phase counterpart of NeoForge
26.3's `RegisterParticleGroupsEvent`. It receives a `RegistrationContext` once at client
startup. Call `event.register` with a namespaced ID and a factory. The factory returns a
`ParticleGroup` with a `ParticleBlend` (`Opaque`, `Translucent`, or `Additive`) and the particle types
that draw in it. `Additive` draws after the translucent layer, depth-tested without depth writes,
blended additively.

```rust,ignore
fn register_particle_groups(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterParticleGroupsEvent<'_>,
) {
    event.register(Identifier::parse("example:glow").unwrap(), || ParticleGroup {
        blend: ParticleBlend::Translucent,
        particles: vec!["example:orbit".into(), "example:spark".into()],
    });
}
```

A group is vanilla's `ParticleRenderType` with the `ParticleGroup` the particle engine keeps for
it. Its particles are counted, admitted, ticked and removed apart from every other group's, so
each group has vanilla's own limit of 16384 live particles and a crowd in one cannot crowd out
another, and they are drawn after the built-in particles, groups in registration order. The group
decides the layer of its particles (`blend`), whatever their sprites hold. List each type by its
namespaced ID, such as `example:spark` for `assets/example/particles/spark.json`. This works with
or without a `register_particle_providers` provider, so a group adds no particle look or motion
of its own: that stays in the declaration and the provider. A type no group lists draws in the
built-in group as before, and the built-in particles are never moved. A type another group lists
already, or no sprite set matches, is logged and left out.

The game freezes factories in registration order. Any nonempty namespace and path is
allowed, including `minecraft`. Duplicate IDs fail client startup without invoking any
factory (matching NeoForge's `RegisterParticleGroupsEvent`). IDs with an empty namespace
or path are logged and skipped. Each particle engine calls the accepted factories once
when it is constructed. If a factory is disabled, refuses or panics, that engine omits
the group. When sprite sets load or reload, the game resolves each type's group from the
resulting engine-owned definitions. A spawn, a tick and a draw use the resolved group
handle, look nothing up by group name and scan no mod. Live provider-driven particles are
dropped when resources reload, in a group as in the built-in one."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_particle_groups::native_register_particle_groups",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "groups",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterParticleGroupsEvent",
        version = 3
    )]
    fn register_particle_groups(groups: &mut Vec<Answered<ParticleGroupRegistration>>);
}
