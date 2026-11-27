//! The `player_commands_sent` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Filter the command tree just before it is sent to a live server player,
    /// including on join, on respawn, on dimension changes and on command refresh.
    /// Use `player` with the host to read the recipient and `tree.hide_where(...)`
    /// to select roots to omit from the permission-filtered tree. Aliases that
    /// redirect to removed commands are omitted too. This only changes command
    /// highlighting and completion on that client; it does not prevent execution. A client
    /// parses the tree it receives to find the arguments it must sign (NeoForge's
    /// `ClientPacketListener.sendCommand` does the same with the client command tree), so
    /// hiding a root that has signable arguments (`msg`, `say`, `me`, `teammsg`) leaves the
    /// player's client unable to sign them: it sends the command unsigned, and a server that
    /// enforces secure chat refuses it. Hide such a root only where that is acceptable; the
    /// server's signature enforcement is not relaxed.
    /// At join the handle has connection data before appearing in online player lists.
    /// A permission update exposes the new level; cross-dimension travel exposes the
    /// source dimension and position until the command tree has been sent.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_commands_sent",
        dispatch = "api_carrier",
        host = "read",
        receiver = "read",
        side = "server",
        contract = "collect",
        out = "answers",
        answered_by = "mod",
        version = 2,
        neoforge = "none (NeoForge 26.3 has no send-time filter event)"
    )]
    fn player_commands_sent(
        player: PlayerRef,
        tree: CommandTreeFilter<'_>,
        answers: &mut dyn FnMut(Answered<CommandTreeSelection>),
    );
}
