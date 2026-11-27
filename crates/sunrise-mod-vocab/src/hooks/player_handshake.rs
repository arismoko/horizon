//! The `player_handshake` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A login or allowed transfer reached the server, after the transfer and network
    /// protocol checks and before the login hello, authentication, or admission.
    /// Runs once on the server thread. There is no player or profile yet.
    ///
    /// `requested_host` is untrusted text, including any suffix after a NUL. Each listener
    /// sees the previous accepted rewrite; an allowed login keeps the final text verbatim,
    /// and `PlayerRef::virtual_host` reports it up to its first NUL.
    /// The 255 UTF-16 code-unit limit from `ClientIntentionPacket` and
    /// `Utf8String.read` applies only to decoding, not to a listener's rewrite.
    /// `requested_port`, `protocol_version` and `peer_ip` are the handshake's own values: each
    /// listener gets its own event with them, so no listener changes what the next one reads.
    /// `peer_ip` is the numeric transport address without a port, or `None` for an
    /// integrated server's memory connection. Rewriting the host does not change the
    /// peer address or bypass the normal login checks.
    ///
    /// Set `cancelled` to refuse login. The connection receives a login disconnect with
    /// `reason`, or the standard localized "Disconnected" text if the reason is unset,
    /// fails Text serialization validation, or is too long for the login disconnect's
    /// reason string (`ClientboundLoginDisconnectPacket`).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_handshake",
        dispatch = "direct",
        side = "server",
        contract = "cancel",
        out = "cancel",
        event_per_listener = true,
        paper = "PlayerHandshakeEvent"
    )]
    fn player_handshake(
        requested_host: &mut String,
        requested_port: u16,
        protocol_version: i32,
        peer_ip: Option<&str>,
        cancel: &mut bool,
        reason: &mut Option<Text>,
    );
}
