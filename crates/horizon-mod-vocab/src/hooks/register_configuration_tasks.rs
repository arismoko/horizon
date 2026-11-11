//! The `register_configuration_tasks` hook.

pub trait Hooks {
    /// Append tasks for a joining profile on the server thread. The profile's
    /// `player` UUID and `name` are known, and `brand` is the client's reported
    /// brand. The player is still configuring and has no player handle.
    /// Register only tasks the peer can handle. Its brand is advisory.
    /// Tasks run in order after the built-in tasks. Registration and the
    /// queue share a 30-second deadline. A task awaiting a reply holds the
    /// connection in configuration until the reply arrives.
    #[hook(
        side = "server",
        contract = "collect",
        out = "tasks",
        neoforge = "RegisterConfigurationTasksEvent"
    )]
    fn register_configuration_tasks();
}
