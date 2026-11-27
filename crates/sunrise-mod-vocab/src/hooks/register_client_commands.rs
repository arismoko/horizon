//! The `register_client_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The client's own commands (NeoForge's `RegisterClientCommandsEvent`): push
    /// each usage with its executor attached by [`Command::executes_client`]
    /// (`Command::new("x <mode:word>").executes_client(...)`).
    /// Asked when a connection starts and again when the server sends its
    /// own tree. Client and server branches can share a root in the chat's
    /// display tree, but run in separate dispatchers. Only an unknown command
    /// or argument in the client dispatcher proceeds to the server.
    /// A usage's `level` is checked against the local player's permission level, as
    /// NeoForge's `ClientCommandHandler.getSource` builds its source with
    /// `player.permissions()` (the level the server's entity events 24 to 28 set): below it,
    /// the usage is neither completed nor run. Server-only handle argument kinds (`player`,
    /// `players`, `entity`, `entities`, `dimension`) are refused.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_register_client_commands",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "commands",
        answered_by = "mod",
        neoforge = "RegisterClientCommandsEvent",
        version = 3
    )]
    fn register_client_commands(
        commands: &mut Vec<mod_api::Answered<mod_api::ClientCommandRegistration>>,
    );
}
