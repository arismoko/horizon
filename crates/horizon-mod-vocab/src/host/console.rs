//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
/// The server host's methods of the `console` area, implemented by the game and reached through [`crate::ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ConsoleServerHost {
    /// Side: server.
    /// Runs `line` (a leading slash is dropped) as the server's console does,
    /// from the overworld's spawn at permission level 4, as
    /// `Commands::run_command`, but keeps its feedback: whether it ran without
    /// failing, and the lines the console would have printed for it, as plain
    /// text (`commands.time.query` and the like, an error's message), for a
    /// mod that answers a remote console (Paper's `Server.dispatchCommand`
    /// with a capturing `CommandSender`). The dedicated console's own
    /// commands (`stop`, `op`, `whitelist`, `ban`...) are not reached.
    /// `(false, [])` where nobody else can join.
    #[access(WorldOnly)]
    fn server_run_command_captured(&mut self, line: &str) -> (bool, Vec<String>);
}
