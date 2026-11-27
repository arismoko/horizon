//! The `register_json_animation_types` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register the custom animation targets and interpolations a mod adds for keyframe animation
    /// files to name (NeoForge's `RegisterJsonAnimationTypesEvent`). Asked once at start on the
    /// client, before resources load. Use [`RegisterJsonAnimationTypesEvent::register_target`] and
    /// [`RegisterJsonAnimationTypesEvent::register_interpolation`] to bind each identifier directly
    /// to its callbacks. Any valid `<namespace>:<path>` identifier (including `minecraft:`) is
    /// accepted; duplicates and collisions with the built-in targets (`minecraft:position`,
    /// `minecraft:rotation`, `minecraft:scale`) or built-in interpolations (`minecraft:linear`,
    /// `minecraft:catmullrom`) are rejected as registration errors.
    #[notes = r#"## Animation types

A keyframe animation file (`assets/<namespace>/neoforge/animations/entity/<path>.json`) names a target per
entry and an interpolation per keyframe: vanilla's `minecraft:position`, `minecraft:rotation` and
`minecraft:scale`, and `minecraft:linear` and `minecraft:catmullrom`. A mod registers its own (NeoForge's
`RegisterJsonAnimationTypesEvent`) once at client startup via `Hooks::register_json_animation_types`,
binding each identifier directly to its callbacks. When animation files are parsed during resource load and
reload, each custom target's converter transforms the raw keyframe vectors into internal units (matching
NeoForge's `AnimationParser`) and binds the target and interpolation callbacks directly onto the parsed
`AnimationHolder`. Baking with `holder.bake(&root)` produces a `KeyframeAnimation` that calls those bound
callbacks directly during playback with no lookup per sample.

```rust,ignore
fn register_json_animation_types(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterJsonAnimationTypesEvent<'_>,
) {
    event.register_target(
        Identifier::parse("example:glow").unwrap(),
        |part, value| part.offset_scale(value),
        |raw| raw.map(|v| v * 0.5),
    );
    event.register_interpolation(
        Identifier::parse("example:step").unwrap(),
        |keyframes, _alpha, prev, _next, scale| keyframes[prev].post.map(|v| v * scale),
    );
}

// In client_setup and client_resources_reloaded, once:
let baked = holder.bake(&root);
// Each frame:
baked.apply(&mut root, &state, age_in_ticks);
```

Any valid `<namespace>:<path>` identifier (including `minecraft:`) is accepted; a target and an
interpolation with the same identifier are separate namespaces. Duplicate registrations and collisions with
the five built-ins (`minecraft:position`, `minecraft:rotation`, `minecraft:scale`, `minecraft:linear`,
`minecraft:catmullrom`) are logged as registration errors and the first registration stands. An animation
file naming an unregistered target or interpolation fails to load, naming it. If a custom target's
converter returns a non-finite vector, that animation file fails to load; if a callback panics during parse
or playback, that callback is disabled for the session with an error log and toast."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_json_animation_types::native_register_json_animation_types",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "collect",
        out = "types",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "RegisterJsonAnimationTypesEvent"
    )]
    fn register_json_animation_types(types: &mut Vec<Answered<JsonAnimationTypeRegistration>>);
}
