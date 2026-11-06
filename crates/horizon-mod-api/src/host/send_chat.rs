//! The `send_chat` area: host methods the game implements (`game/host/send_chat.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `send_chat` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait SendChatClientHost {
    /// Side: client.
    /// A mod's chat line or command (`/` first), sent as `send_chat` sends
    /// it (ClientPacketListener.sendChat / sendCommand), to any server: no
    /// chat history, and no client chat event (vanilla's `ClientChatEvent`
    /// comes only from ChatScreen). An error when the player isn't in a
    /// server's play state or the Chat option blocks the message. API
    /// 404.
    fn send_chat(&mut self, message: &str) -> Result<(), String> {
        crate::phase::unprovided(
            "ClientHost::send_chat",
            "send_chat",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
