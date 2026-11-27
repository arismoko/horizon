//! The `component_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Register namespaced NBT component types before content loads. Call
    /// [`ComponentTypesEvent::register`] with each explicit `namespace:path`, such as
    /// `"mymod:charge".into()`. Invalid and duplicate IDs, including IDs already used by
    /// built-in components, are refused before the component registry freezes.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::native_component_types",
        receiver = "associated",
        dispatch = "flat",
        side = "both",
        contract = "collect",
        out = "types",
        answered_by = "mod",
        registration_append = "register",
        context = "registration",
        neoforge = "RegisterEvent",
        version = 4
    )]
    fn component_types(types: &mut Vec<Answered<String>>);
}
