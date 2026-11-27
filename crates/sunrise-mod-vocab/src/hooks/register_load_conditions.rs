//! Load-phase data-file condition registration.
pub trait Hooks {
    /// Register named conditions once at mod load, before server data admission.
    /// Each handler is a raw-JSON handler: it reads the condition's JSON value.
    /// IDs may use any namespace, but a listener that completes with a built-in or duplicate
    /// ID error refuses mod load, even if it ignored the returned error. A listener that
    /// panics or incurs a phase refusal publishes neither handlers nor preparation errors.
    /// Whether an unknown ID refuses a load or skips a file is determined by the
    /// loader, not the namespace.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_load_conditions::native_register_load_conditions",
        receiver = "associated",
        dispatch = "flat",
        side = "server",
        version = 3,
        contract = "listen",
        staged_surface = "conditions",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn register_load_conditions(conditions: &mut dyn LoadConditions);
}
