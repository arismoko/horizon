//! The `client_resources_reloaded` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not
    /// (`client_setup` is the once-only moment after the initial load). Receives a
    /// [`ClientReloadHost`] for published asset/data queries; world handles and
    /// transactions are not present on this context.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::native_client_resources_reloaded",
        receiver = "associated",
        context = "client_reload",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        neoforge = "ClientResourceLoadFinishedEvent",
        version = 2
    )]
    fn client_resources_reloaded();
}
