//! The `tooltip_image` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Optional tooltip component data for this item stack (`Item.getTooltipImage`).
    /// Answer `Some(TooltipComponent::custom(data))` (or `Some(TooltipComponent::bundle(stack))`)
    /// to insert a component after the item's name line before
    /// `render_tooltip_gather_components` runs. Register a client factory for custom data types
    /// in `register_client_tooltip_component_factories`. Answer `None` to leave the vanilla
    /// tooltip component unchanged. A callback or retained payload destructor panic
    /// disables only this item-part registration for this hook. Client-side.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_tooltip_image",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 2,
        contract = "replace",
        out = "component",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.getTooltipImage)"
    )]
    fn tooltip_image(stack: &ItemStack, component: &mut Option<mod_api::TooltipComponent>);
}
