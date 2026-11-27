//! Client dimension-loading screen registration.

pub trait Hooks {
    /// Register loading-screen factories once at client startup (NeoForge's
    /// `RegisterDimensionTransitionScreenEvent`). Each mod receives an empty batch.
    /// Conditional pairs take priority over incoming, then outgoing. The first valid
    /// factory for a key wins. Unmatched travel, login and death use vanilla for a new screen.
    /// The client keeps whichever presentation is already open on an in-place load update.
    #[notes = r#"## Dimension transition screens

Push `DimensionTransitionScreen::new(transition, factory)` into `event.screens`.
Use `DimensionTransition::Incoming(to)`, `Outgoing(from)` or `Conditional { to, from }`
with namespaced dimension IDs. The client resolves those IDs at registration and logs and
rejects invalid or duplicate transition keys.

On a matching trip, the client calls the selected factory once as the registering mod and
keeps its presentation until the loading screen closes. An existing loading screen (including
vanilla's) keeps its presentation if another level load starts while it is open. Login and
death respawn do not select factories when they open a new loading screen. Return a boxed
`DimensionTransitionPresentation` (a draw closure or an object); each draw receives the current
reason and optional smoothed integrated-server progress. The existing tracker still governs
readiness, timeout and closing."#]
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::register_dimension_transition_screen::native_register_dimension_transition_screen",
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "screens",
        context = "registration",
        answered_by = "mod",
        neoforge = "RegisterDimensionTransitionScreenEvent",
        version = 3
    )]
    fn register_dimension_transition_screen(
        screens: &mut Vec<mod_api::Answered<mod_api::DimensionTransitionScreen>>,
    );
}
