//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `network` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait NetworkClientHost {
    /// Side: client.
    /// Opens the chat screen with `text` typed in for the player to finish and
    /// send; nothing is sent. It can replace an open screen, including another
    /// chat screen or foreground prompt. A new chat instance fires normal opening,
    /// closing and initialization hooks; a veto leaves the old screen untouched.
    /// Returns an error when there is no local player.
    fn open_chat(&mut self, text: &str) -> Result<(), String>;
    /// Side: client.
    /// Adds `line` to this client's chat HUD and history (vanilla's
    /// `ChatComponent.addClientSystemMessage`): it shows here only, so
    /// nothing is sent to the server. The Chat option's server-message filter
    /// doesn't hide it (`GuiMessageSource.SYSTEM_CLIENT`), and
    /// `client_chat_received` is not told: vanilla fires that event from
    /// `ChatListener` for lines the server sent, not for lines the client adds
    /// itself. Returns an error if the text exceeds the boundary's size or
    /// nesting limits; rejected text is not added.
    fn chat_line(&mut self, line: Text) -> Result<(), TextFormError>;
    /// Side: client.
    /// `chat_line` for a status line the calling mod keeps current: lines are named by `tag`,
    /// a key of the calling mod's own (another mod's equal `tag` is another line). When this
    /// mod already has a line under `tag` in the chat, its text becomes `line` in place: the
    /// line keeps its position among the messages, counts as new for the HUD's fade, and the
    /// chat wraps its lines again (`ChatComponent.refreshTrimmedMessages`). Otherwise `line` is
    /// added as `chat_line` adds it. To move the line to the bottom, call
    /// [`chat_remove_tagged`](Self::chat_remove_tagged) first. A line that left the history
    /// (the cap, a clear) is no longer found, so the next call adds a new one. Outside a mod's
    /// own call it is `chat_line`. Returns an error, adding or changing nothing, if the text
    /// exceeds the boundary's size or nesting limits.
    fn chat_line_tagged(&mut self, tag: &str, line: Text) -> Result<(), TextFormError>;
    /// Side: client.
    /// Removes the calling mod's lines under `tag` from the chat and its history, rewrapping what
    /// is left (vanilla's `ChatComponent.deleteMessage` greys a signed message out; a mod's own
    /// line just goes). Returns how many lines went: 0 when there were none. Another mod's lines
    /// and lines added with `chat_line` are never removed.
    fn chat_remove_tagged(&mut self, tag: &str) -> usize;
    /// Side: client.
    /// How many messages, wrapped lines and sent lines (`recentChat`) the chat keeps
    /// (`ChatComponent.MAX_CHAT_HISTORY`): vanilla's 100 unless a mod set another cap.
    fn chat_history_limit(&self) -> u32;
    /// Side: client.
    /// Sets the chat history cap for this session, as chat-history mods raise
    /// `ChatComponent.MAX_CHAT_HISTORY`, clamped to 1 to 16384. A lower cap trims the oldest
    /// messages, wrapped lines and sent lines now; a higher one lets later ones gather up to it.
    /// The saved command history (`command_history.txt`, 50 commands) keeps its own cap.
    fn set_chat_history_limit(&mut self, limit: u32);
    /// Side: client.
    /// Sends directly through `ClientPacketListener.sendChat` or `sendCommand`
    /// (a leading `/` selects the command path). Chat passes through `client_chat`
    /// before signing; commands run the local dispatcher before being sent to the server.
    /// This does not normalize input, check the client's Chat setting or add history.
    /// `Ok(())` means queued, handled locally or intentionally canceled; `Err` means
    /// the play listener was unavailable or local encoding or publication failed.
    /// The server still checks signatures, permissions and chat policy. API 404.
    fn send_chat(&mut self, message: &str) -> Result<(), String>;
    /// Side: client.
    /// Encodes a typed serverbound play packet with the client's vanilla protocol encoder.
    /// Available in client host contexts including setup, without reading a connection or
    /// changing local state. Validates vanilla wire field bounds and applies no gameplay or
    /// local effects; submit the result with `send_packet`.
    #[static_call]
    fn build_play_packet(
        &self,
        request: &PlayPacketRequest,
    ) -> Result<ServerboundPlayPacket, PacketBuildError>;
    /// Side: client.
    /// Builds a vanilla container click; it needs the host because the packet carries hashed stacks.
    /// Uses this connection's registries and component codecs, refusing components that
    /// cannot be encoded or hashed, unrepresentable IDs/slots and more than 128 changed slots.
    /// Applies no menu checks or local effects; submit the result with `send_packet`.
    fn container_click(
        &self,
        request: &ContainerClickRequest,
    ) -> Result<ServerboundPlayPacket, PacketBuildError>;
    /// Side: client.
    /// Submits a vanilla play packet to any server, including unrelated plugin
    /// payloads. The body is validated once and outgoing credit is reserved before
    /// acceptance. At root commit the packet joins any active play holds without
    /// releasing its credit, otherwise it publishes. Rollback, disconnect or a
    /// protocol incarnation change cancels it.
    /// Registered and engine-reserved channels are refused: use their payload or
    /// service API. Unrelated plugin bodies retain vanilla's discard-codec limit.
    /// No effects are applied locally. Typed packet builders validate wire fields;
    /// sequence numbers and required client state remain the mod's responsibility.
    /// Chat and commands use `send_chat`, which owns signing. API 409.
    fn send_packet(&mut self, packet: ServerboundPlayPacket) -> Result<(), SendPacketError>;
    /// Side: client. Starts a hold owned by the calling mod on the current joined
    /// physical play connection. It covers subsequent uncancelled engine play sends
    /// and mod packets committed while it is active. Holds can overlap. A pending
    /// packet retains its ordinary writer credit; pressure never releases a hold.
    /// Control replies (including keep-alives, chunk batch acknowledgements and cookies)
    /// bypass holds. Disconnect, protocol changes and aborted travel invalidate the id.
    fn hold_packets(&mut self) -> Result<HoldId, HoldError>;
    /// Side: client. Ends only this mod's hold. Publishes the newly unblocked prefix
    /// in original send order and returns its count, which can be zero. Other holds
    /// still block their packets. `Send` reports a published prefix on transport failure.
    fn release_held(&mut self, hold: HoldId) -> Result<usize, HoldError>;
    /// Side: client. Ends only this mod's hold and discards packets it covered,
    /// including overlapping packets. It publishes nothing and keeps the other holds active.
    fn drop_held(&mut self, hold: HoldId) -> Result<(), HoldError>;
    /// Side: client.
    /// Leaves the server or the single-player world (NeoForge's
    /// `IClientCommonPacketListenerExtension.disconnect(Component)`), at the end of this frame,
    /// not inside the calling hook. With a reason, the disconnect screen shows it (a
    /// single-player world is saved first); with none, it leaves as the pause menu's Disconnect
    /// does. The request belongs to the world it was made in: one made while that world closes
    /// is dropped, never carried into the next. A second call in the same frame replaces the
    /// reason. The retained reason renders in the active language at draw and narration time;
    /// literal text stays unchanged. `DisconnectError::Unavailable` when no server or world is joined.
    /// The reason is local to the client, not encoded into a packet.
    fn disconnect(&mut self, reason: Option<Text>) -> Result<(), DisconnectError>;
}
