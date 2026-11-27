//! The `register_client_tooltip_component_factories` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register client tooltip component factories for custom `TooltipComponentData` types
    /// (NeoForge's `RegisterClientTooltipComponentFactoriesEvent`). Asked once at client
    /// startup. Call `event.factories.register::<MyData, MyClientComponent>(|data| ...)` to
    /// map each custom `TooltipComponent` produced by `ItemPart::tooltip_image` or
    /// `render_tooltip_gather_components` into a `ClientTooltipComponent` that measures and
    /// draws itself. A later registration for the same type replaces the earlier one. A listener
    /// that panics leaves all previously registered factories in place and publishes none of its own.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::register_client_tooltip_component_factories::native_register_client_tooltip_component_factories",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        staged_surface = "factories",
        context = "registration",
        neoforge = "RegisterClientTooltipComponentFactoriesEvent"
    )]
    fn register_client_tooltip_component_factories(
        factories: &mut dyn mod_api::ClientTooltipComponentFactoriesRegistrar,
    );
}
