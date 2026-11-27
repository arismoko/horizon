//! The `register_fluid_models` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The models of your world fluids (NeoForge's `RegisterFluidModelsEvent`): call
    /// `event.register(model)` with a `FluidModel` per fluid whose look doesn't say all of it. Its
    /// sprites are block atlas textures, baked at every resource load from that pack's files; the
    /// tint stays the look's. A fluid without one draws its look's sprite on every surface, or
    /// water's textures when it has none. Asked at startup and on resource reload before baking, and
    /// only on the client.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_fluid_models::native_register_fluid_models",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "models",
        registration_append = "register",
        context = "registration",
        neoforge = "RegisterFluidModelsEvent",
        version = 3
    )]
    fn register_fluid_models(models: &mut Vec<mod_api::FluidModel>);
}
