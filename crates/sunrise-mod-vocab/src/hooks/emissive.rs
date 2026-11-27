//! The `emissive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register emissive overlay sprite pairs (`EmissiveSpriteRegistration`) at client startup
    /// and on every resource reload, before terrain baking finishes. Call `event.register(pair)`
    /// with typed base and overlay `Identifier`s; per-pixel alpha in the overlay marks
    /// full-bright material over the base sprite without illuminating surrounding blocks or
    /// changing gameplay light. Resolved pairs stay bound to the loaded render-asset generation
    /// until the next reload and share the GPU emissive uniform table with OptiFine overlays.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::native_emissive",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "layers",
        answered_by = "mod",
        registration_append = "register",
        context = "registration",
        neoforge = "none (Sunrise-only; NeoForge has no counterpart)",
        version = 2
    )]
    fn emissive(layers: &mut Vec<Answered<EmissiveSpriteRegistration>>);
}
